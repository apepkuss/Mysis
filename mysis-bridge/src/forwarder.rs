use mysis_core::protocol::*;

use crate::config::LlmConfig;

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

    /// 转发 LLM 请求到 ironmlx，返回 Mysis 格式的响应
    pub async fn forward(&self, req: &LlmRequest) -> Result<LlmResponse, String> {
        let url = format!("{}/chat/completions", self.config.base_url);
        let body = build_openai_request(req, &self.config.model);

        let http_resp = self
            .client
            .post(&url)
            .json(&body)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_openai_request_body() {
        let mysis_req = LlmRequest {
            id: "req-001".into(),
            messages: vec![Message {
                role: "user".into(),
                content: "hello".into(),
                tool_calls: vec![],
                tool_call_id: None,
            }],
            tools: vec![ToolDefinition {
                r#type: "function".into(),
                function: FunctionDefinition {
                    name: "gpio_write".into(),
                    description: "Write GPIO".into(),
                    parameters: serde_json::json!({"type": "object"}),
                },
            }],
            max_tokens: 100,
        };
        let body = build_openai_request(&mysis_req, "qwen3-8b");
        assert_eq!(body["model"], "qwen3-8b");
        assert_eq!(body["max_tokens"], 100);
        assert_eq!(body["messages"][0]["role"], "user");
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
}
