use mysis_core::protocol::*;

use crate::config::LlmConfig;

// ============================================================
// OpenAI 格式适配
// ============================================================

/// 将 Mysis LlmRequest 转为 OpenAI API 请求 body
pub fn build_openai_request(req: &LlmRequest, model: &str) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "messages": req.messages.iter().map(|m| {
            let mut msg = serde_json::json!({
                "role": m.role,
                "content": m.content,
            });
            if !m.tool_calls.is_empty() {
                msg["tool_calls"] = serde_json::to_value(&m.tool_calls).unwrap_or_default();
            }
            if let Some(ref id) = m.tool_call_id {
                msg["tool_call_id"] = serde_json::Value::String(id.clone());
            }
            msg
        }).collect::<Vec<_>>(),
        "max_tokens": req.max_tokens,
    });

    if !req.tools.is_empty() {
        body["tools"] = serde_json::to_value(&req.tools).unwrap_or_default();
    }

    body
}

/// 将 OpenAI API 响应转为 Mysis LlmResponse
pub fn parse_openai_response(
    request_id: &str,
    openai_resp: &serde_json::Value,
) -> Result<LlmResponse, String> {
    let choice = openai_resp["choices"]
        .as_array()
        .and_then(|arr| arr.first())
        .ok_or("no choices in response")?;

    let message = &choice["message"];
    let finish_reason = choice["finish_reason"]
        .as_str()
        .unwrap_or("stop")
        .to_string();

    let content = message["content"].as_str().map(|s| s.to_string());

    let tool_calls = if let Some(calls) = message["tool_calls"].as_array() {
        calls
            .iter()
            .map(|tc| {
                let func = &tc["function"];
                let arguments_str = func["arguments"].as_str().unwrap_or("{}");
                let arguments: serde_json::Value = serde_json::from_str(arguments_str)
                    .unwrap_or(serde_json::Value::Object(Default::default()));
                ToolCall {
                    id: tc["id"].as_str().unwrap_or("").to_string(),
                    name: func["name"].as_str().unwrap_or("").to_string(),
                    arguments,
                }
            })
            .collect()
    } else {
        vec![]
    };

    Ok(LlmResponse {
        id: request_id.to_string(),
        content,
        tool_calls,
        finish_reason,
    })
}

// ============================================================
// Claude (Anthropic Messages API) 格式适配
// ============================================================

/// 将 Mysis LlmRequest 转为 Claude Messages API 请求 body
pub fn build_claude_request(req: &LlmRequest, model: &str) -> serde_json::Value {
    // 提取 system prompt（Claude 使用顶层 system 字段）
    let system_prompt: Option<String> = req
        .messages
        .iter()
        .find(|m| m.role == "system")
        .map(|m| m.content.clone());

    // 转换消息（跳过 system，转换 tool 结果格式）
    let messages: Vec<serde_json::Value> = req
        .messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            if m.role == "assistant" && !m.tool_calls.is_empty() {
                // assistant 消息带工具调用 → Claude content 数组
                let mut content_blocks: Vec<serde_json::Value> = Vec::new();
                if !m.content.is_empty() {
                    content_blocks.push(serde_json::json!({
                        "type": "text",
                        "text": m.content,
                    }));
                }
                for tc in &m.tool_calls {
                    content_blocks.push(serde_json::json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.name,
                        "input": tc.arguments,
                    }));
                }
                serde_json::json!({
                    "role": "assistant",
                    "content": content_blocks,
                })
            } else if m.role == "tool" {
                // tool 结果 → Claude user 消息中的 tool_result block
                serde_json::json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": m.tool_call_id.as_deref().unwrap_or(""),
                        "content": m.content,
                    }],
                })
            } else {
                serde_json::json!({
                    "role": m.role,
                    "content": m.content,
                })
            }
        })
        .collect();

    // 合并相邻的同 role 消息（Claude 要求 user/assistant 交替）
    let messages = merge_adjacent_messages(messages);

    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "max_tokens": req.max_tokens,
    });

    if let Some(system) = system_prompt {
        body["system"] = serde_json::Value::String(system);
    }

    // 转换工具定义为 Claude 格式
    if !req.tools.is_empty() {
        let tools: Vec<serde_json::Value> = req
            .tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.function.name,
                    "description": t.function.description,
                    "input_schema": t.function.parameters,
                })
            })
            .collect();
        body["tools"] = serde_json::Value::Array(tools);
    }

    body
}

/// 合并相邻的同 role 消息（Claude 要求 user/assistant 严格交替）
fn merge_adjacent_messages(messages: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    let mut merged: Vec<serde_json::Value> = Vec::new();

    for msg in messages {
        let role = msg["role"].as_str().unwrap_or("").to_string();
        let should_merge = merged
            .last()
            .and_then(|last| last["role"].as_str())
            .map(|last_role| last_role == role)
            .unwrap_or(false);

        if should_merge {
            // 合并 content 到上一条消息
            let last = merged.last_mut().unwrap();
            let existing = last["content"].clone();
            let new_content = msg["content"].clone();

            // 统一为数组格式合并
            let mut blocks: Vec<serde_json::Value> = match existing {
                serde_json::Value::Array(arr) => arr,
                serde_json::Value::String(s) => {
                    vec![serde_json::json!({"type": "text", "text": s})]
                }
                other => vec![other],
            };
            match new_content {
                serde_json::Value::Array(arr) => blocks.extend(arr),
                serde_json::Value::String(s) => {
                    blocks.push(serde_json::json!({"type": "text", "text": s}))
                }
                other => blocks.push(other),
            };
            last["content"] = serde_json::Value::Array(blocks);
        } else {
            merged.push(msg);
        }
    }

    merged
}

/// 将 Claude Messages API 响应转为 Mysis LlmResponse
pub fn parse_claude_response(
    request_id: &str,
    claude_resp: &serde_json::Value,
) -> Result<LlmResponse, String> {
    let content_blocks = claude_resp["content"]
        .as_array()
        .ok_or("no content in Claude response")?;

    let stop_reason = claude_resp["stop_reason"]
        .as_str()
        .unwrap_or("end_turn")
        .to_string();

    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    for block in content_blocks {
        match block["type"].as_str() {
            Some("text") => {
                if let Some(text) = block["text"].as_str() {
                    text_parts.push(text.to_string());
                }
            }
            Some("tool_use") => {
                tool_calls.push(ToolCall {
                    id: block["id"].as_str().unwrap_or("").to_string(),
                    name: block["name"].as_str().unwrap_or("").to_string(),
                    arguments: block["input"].clone(),
                });
            }
            _ => {}
        }
    }

    let content = if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join(""))
    };

    // 映射 stop_reason 到 Mysis 统一格式
    let finish_reason = match stop_reason.as_str() {
        "end_turn" => "stop".to_string(),
        "tool_use" => "tool_calls".to_string(),
        other => other.to_string(),
    };

    Ok(LlmResponse {
        id: request_id.to_string(),
        content,
        tool_calls,
        finish_reason,
    })
}

// ============================================================
// LLM 转发器
// ============================================================

pub struct LlmForwarder {
    client: reqwest::Client,
    config: LlmConfig,
}

impl LlmForwarder {
    pub fn new(config: LlmConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .expect("failed to create HTTP client");
        Self { client, config }
    }

    /// 转发 LLM 请求，根据 provider 选择 OpenAI 或 Claude 格式
    pub async fn forward(&self, req: &LlmRequest) -> Result<LlmResponse, String> {
        match self.config.provider.as_str() {
            "claude" => self.forward_claude(req).await,
            _ => self.forward_openai(req).await,
        }
    }

    async fn forward_openai(&self, req: &LlmRequest) -> Result<LlmResponse, String> {
        let url = format!("{}/chat/completions", self.config.base_url);
        let body = build_openai_request(req, &self.config.model);

        let mut http_req = self.client.post(&url).json(&body);
        if let Some(ref api_key) = self.config.api_key {
            http_req = http_req.bearer_auth(api_key);
        }

        let http_resp = http_req
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?;

        if !http_resp.status().is_success() {
            let status = http_resp.status();
            let text = http_resp.text().await.unwrap_or_default();
            return Err(format!("LLM API error {status}: {text}"));
        }

        let openai_resp: serde_json::Value = http_resp
            .json()
            .await
            .map_err(|e| format!("failed to parse LLM response: {e}"))?;

        parse_openai_response(&req.id, &openai_resp)
    }

    async fn forward_claude(&self, req: &LlmRequest) -> Result<LlmResponse, String> {
        let url = format!("{}/v1/messages", self.config.base_url);
        let body = build_claude_request(req, &self.config.model);

        let api_key = self
            .config
            .api_key
            .as_deref()
            .ok_or("api_key is required for Claude provider")?;

        let http_resp = self
            .client
            .post(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?;

        if !http_resp.status().is_success() {
            let status = http_resp.status();
            let text = http_resp.text().await.unwrap_or_default();
            return Err(format!("Claude API error {status}: {text}"));
        }

        let claude_resp: serde_json::Value = http_resp
            .json()
            .await
            .map_err(|e| format!("failed to parse Claude response: {e}"))?;

        parse_claude_response(&req.id, &claude_resp)
    }
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> LlmRequest {
        LlmRequest {
            id: "req-001".into(),
            messages: vec![
                Message {
                    role: "system".into(),
                    content: "You are a smart home assistant.".into(),
                    tool_calls: vec![],
                    tool_call_id: None,
                },
                Message {
                    role: "user".into(),
                    content: "开灯".into(),
                    tool_calls: vec![],
                    tool_call_id: None,
                },
            ],
            tools: vec![ToolDefinition {
                r#type: "function".into(),
                function: FunctionDefinition {
                    name: "gpio_write".into(),
                    description: "Write GPIO".into(),
                    parameters: serde_json::json!({"type": "object"}),
                },
            }],
            max_tokens: 256,
        }
    }

    // --- OpenAI 测试 ---

    #[test]
    fn build_openai_request_body() {
        let req = sample_request();
        let body = build_openai_request(&req, "qwen3-8b");
        assert_eq!(body["model"], "qwen3-8b");
        assert_eq!(body["max_tokens"], 256);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["tools"][0]["type"], "function");
    }

    #[test]
    fn parse_openai_response_text() {
        let openai_json = serde_json::json!({
            "id": "chatcmpl-123",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "灯已打开"
                },
                "finish_reason": "stop"
            }]
        });
        let resp = parse_openai_response("req-001", &openai_json).unwrap();
        assert_eq!(resp.id, "req-001");
        assert_eq!(resp.content.as_deref(), Some("灯已打开"));
        assert!(resp.tool_calls.is_empty());
        assert_eq!(resp.finish_reason, "stop");
    }

    #[test]
    fn parse_openai_response_tool_calls() {
        let openai_json = serde_json::json!({
            "id": "chatcmpl-456",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc",
                        "type": "function",
                        "function": {
                            "name": "gpio_write",
                            "arguments": "{\"pin\":\"light\",\"value\":true}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });
        let resp = parse_openai_response("req-002", &openai_json).unwrap();
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].id, "call_abc");
        assert_eq!(resp.tool_calls[0].name, "gpio_write");
        assert_eq!(resp.tool_calls[0].arguments["pin"], "light");
        assert_eq!(resp.finish_reason, "tool_calls");
    }

    // --- Claude 测试 ---

    #[test]
    fn build_claude_request_body() {
        let req = sample_request();
        let body = build_claude_request(&req, "claude-sonnet-4-20250514");
        assert_eq!(body["model"], "claude-sonnet-4-20250514");
        assert_eq!(body["max_tokens"], 256);
        // system 应提取到顶层
        assert_eq!(body["system"], "You are a smart home assistant.");
        // messages 不应包含 system
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "开灯");
        // 工具定义为 Claude 格式
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools[0]["name"], "gpio_write");
        assert!(tools[0].get("input_schema").is_some());
        assert!(tools[0].get("function").is_none());
    }

    #[test]
    fn build_claude_request_with_tool_calls() {
        let req = LlmRequest {
            id: "req-002".into(),
            messages: vec![
                Message {
                    role: "user".into(),
                    content: "开灯".into(),
                    tool_calls: vec![],
                    tool_call_id: None,
                },
                Message {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "toolu_01".into(),
                        name: "gpio_write".into(),
                        arguments: serde_json::json!({"pin": "light", "value": true}),
                    }],
                    tool_call_id: None,
                },
                Message {
                    role: "tool".into(),
                    content: r#"{"success":true}"#.into(),
                    tool_calls: vec![],
                    tool_call_id: Some("toolu_01".into()),
                },
            ],
            tools: vec![],
            max_tokens: 256,
        };
        let body = build_claude_request(&req, "claude-sonnet-4-20250514");
        let messages = body["messages"].as_array().unwrap();

        // user
        assert_eq!(messages[0]["role"], "user");
        // assistant with tool_use
        assert_eq!(messages[1]["role"], "assistant");
        let assistant_content = messages[1]["content"].as_array().unwrap();
        assert_eq!(assistant_content[0]["type"], "tool_use");
        assert_eq!(assistant_content[0]["name"], "gpio_write");
        // tool_result 合并到 user 消息
        assert_eq!(messages[2]["role"], "user");
        let user_content = messages[2]["content"].as_array().unwrap();
        assert_eq!(user_content[0]["type"], "tool_result");
        assert_eq!(user_content[0]["tool_use_id"], "toolu_01");
    }

    #[test]
    fn parse_claude_response_text() {
        let claude_json = serde_json::json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "content": [
                {"type": "text", "text": "客厅灯已打开"}
            ],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 50, "output_tokens": 10}
        });
        let resp = parse_claude_response("req-001", &claude_json).unwrap();
        assert_eq!(resp.content.as_deref(), Some("客厅灯已打开"));
        assert!(resp.tool_calls.is_empty());
        assert_eq!(resp.finish_reason, "stop");
    }

    #[test]
    fn parse_claude_response_tool_use() {
        let claude_json = serde_json::json!({
            "id": "msg_456",
            "type": "message",
            "role": "assistant",
            "content": [
                {"type": "text", "text": "好的，我来开灯。"},
                {
                    "type": "tool_use",
                    "id": "toolu_01A",
                    "name": "gpio_write",
                    "input": {"pin": "living_room_light", "value": true}
                }
            ],
            "stop_reason": "tool_use"
        });
        let resp = parse_claude_response("req-002", &claude_json).unwrap();
        assert_eq!(resp.content.as_deref(), Some("好的，我来开灯。"));
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].id, "toolu_01A");
        assert_eq!(resp.tool_calls[0].name, "gpio_write");
        assert_eq!(resp.tool_calls[0].arguments["pin"], "living_room_light");
        assert_eq!(resp.finish_reason, "tool_calls");
    }

    #[test]
    fn merge_adjacent_user_messages() {
        let messages = vec![
            serde_json::json!({"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "ok"}]}),
            serde_json::json!({"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t2", "content": "ok"}]}),
        ];
        let merged = merge_adjacent_messages(messages);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0]["content"].as_array().unwrap().len(), 2);
    }
}
