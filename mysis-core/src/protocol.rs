use serde::{Deserialize, Serialize};

/// LLM 请求（ESP32 → Bridge）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequest {
    pub id: String,
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

fn default_max_tokens() -> u32 {
    256
}

/// 消息（OpenAI 兼容格式）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    /// 仅 assistant 消息：LLM 返回的工具调用列表
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// 仅 tool 消息：对应的 tool_call id
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// 工具定义（OpenAI 兼容 JSON Schema 格式）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub r#type: String,
    pub function: FunctionDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// LLM 响应（Bridge → ESP32）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub id: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: String,
}

/// 工具调用
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// 设备心跳（ESP32 → Bridge）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heartbeat {
    pub device_id: String,
    pub uptime_secs: u64,
    pub free_heap: u64,
    pub wifi_rssi: i32,
    pub tools: Vec<String>,
}

/// 主动命令（Bridge → ESP32）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    pub id: String,
    pub action: String,
    pub tool: String,
    pub arguments: serde_json::Value,
}

/// 记忆存储请求（ESP32 → Bridge）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStoreRequest {
    pub id: String,
    pub category: String, // preference / fact / event
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// 记忆召回请求（ESP32 → Bridge）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecallRequest {
    pub id: String,
    pub query: String,
    #[serde(default = "default_recall_limit")]
    pub limit: u32,
}

fn default_recall_limit() -> u32 {
    5
}

/// 记忆召回结果（Bridge → ESP32）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecallResult {
    pub id: String,
    pub memories: Vec<MemoryEntry>,
}

/// 单条记忆条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub category: String,
    pub content: String,
    #[serde(default)]
    pub relevance: f32,
}

/// 冷启动恢复响应（Bridge → ESP32）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySyncResponse {
    pub id: String,
    pub preferences: Vec<MemoryPreference>,
    pub summary: String,
}

/// 偏好键值对
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryPreference {
    pub key: String,
    pub value: String,
}

/// MQTT 主题构建辅助
pub struct Topics;

impl Topics {
    pub fn llm_request(device_id: &str) -> String {
        format!("mysis/{device_id}/llm/request")
    }

    pub fn llm_response(device_id: &str) -> String {
        format!("mysis/{device_id}/llm/response")
    }

    pub fn status(device_id: &str) -> String {
        format!("mysis/{device_id}/status")
    }

    pub fn command(device_id: &str) -> String {
        format!("mysis/{device_id}/command")
    }

    pub fn memory_store(device_id: &str) -> String {
        format!("mysis/{device_id}/memory/store")
    }

    pub fn memory_recall(device_id: &str) -> String {
        format!("mysis/{device_id}/memory/recall")
    }

    pub fn memory_result(device_id: &str) -> String {
        format!("mysis/{device_id}/memory/result")
    }

    pub fn memory_sync(device_id: &str) -> String {
        format!("mysis/{device_id}/memory/sync")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_llm_request() {
        let req = LlmRequest {
            id: "req-001".into(),
            messages: vec![Message {
                role: "user".into(),
                content: "把客厅灯打开".into(),
                tool_calls: vec![],
                tool_call_id: None,
            }],
            tools: vec![],
            max_tokens: 256,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("req-001"));
        assert!(json.contains("把客厅灯打开"));
    }

    #[test]
    fn deserialize_llm_response_text() {
        let json = r#"{"id":"req-001","content":"好的","tool_calls":[],"finish_reason":"stop"}"#;
        let resp: LlmResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, "req-001");
        assert_eq!(resp.content.as_deref(), Some("好的"));
        assert!(resp.tool_calls.is_empty());
        assert_eq!(resp.finish_reason, "stop");
    }

    #[test]
    fn deserialize_llm_response_tool_calls() {
        let json = r#"{
            "id": "req-001",
            "content": null,
            "tool_calls": [
                {"id": "call_001", "name": "gpio_write", "arguments": {"pin": "living_room_light", "value": true}}
            ],
            "finish_reason": "tool_calls"
        }"#;
        let resp: LlmResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].id, "call_001");
        assert_eq!(resp.tool_calls[0].name, "gpio_write");
    }

    #[test]
    fn serialize_heartbeat() {
        let hb = Heartbeat {
            device_id: "mysis-living-room".into(),
            uptime_secs: 3600,
            free_heap: 245760,
            wifi_rssi: -45,
            tools: vec!["gpio_write".into(), "gpio_read".into()],
        };
        let json = serde_json::to_string(&hb).unwrap();
        assert!(json.contains("mysis-living-room"));
        assert!(json.contains("3600"));
    }

    #[test]
    fn deserialize_command() {
        let json = r#"{"id":"cmd-001","action":"execute_tool","tool":"gpio_write","arguments":{"pin":"living_room_light","value":false}}"#;
        let cmd: Command = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.id, "cmd-001");
        assert_eq!(cmd.tool, "gpio_write");
    }

    #[test]
    fn serialize_memory_store_request() {
        let req = MemoryStoreRequest {
            id: "mem-001".into(),
            category: "preference".into(),
            content: "用户说'灯'时默认指客厅灯".into(),
            metadata: Some(serde_json::json!({
                "key": "default_light",
                "value": "living_room_light"
            })),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("mem-001"));
        assert!(json.contains("preference"));
    }

    #[test]
    fn serialize_memory_recall_request() {
        let req = MemoryRecallRequest {
            id: "mem-002".into(),
            query: "浇花 时间".into(),
            limit: 3,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("浇花 时间"));
        assert!(json.contains("3"));
    }

    #[test]
    fn deserialize_memory_recall_result() {
        let json = r#"{
            "id": "mem-002",
            "memories": [
                {"category": "event", "content": "2026-03-15 浇花", "relevance": 0.92}
            ]
        }"#;
        let result: MemoryRecallResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.memories.len(), 1);
        assert_eq!(result.memories[0].category, "event");
    }

    #[test]
    fn serialize_memory_sync_response() {
        let resp = MemorySyncResponse {
            id: "mem-003".into(),
            preferences: vec![MemoryPreference {
                key: "default_light".into(),
                value: "living_room_light".into(),
            }],
            summary: "用户常在晚上8点开客厅灯".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("default_light"));
        assert!(json.contains("living_room_light"));
    }

    #[test]
    fn memory_topics() {
        assert_eq!(Topics::memory_store("dev-01"), "mysis/dev-01/memory/store");
        assert_eq!(
            Topics::memory_recall("dev-01"),
            "mysis/dev-01/memory/recall"
        );
        assert_eq!(
            Topics::memory_result("dev-01"),
            "mysis/dev-01/memory/result"
        );
        assert_eq!(Topics::memory_sync("dev-01"), "mysis/dev-01/memory/sync");
    }

    #[test]
    fn tool_definition_roundtrip() {
        let tool = ToolDefinition {
            r#type: "function".into(),
            function: FunctionDefinition {
                name: "gpio_write".into(),
                description: "控制 GPIO 引脚输出".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "pin": {"type": "string"},
                        "value": {"type": "boolean"}
                    },
                    "required": ["pin", "value"]
                }),
            },
        };
        let json = serde_json::to_string(&tool).unwrap();
        let parsed: ToolDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.function.name, "gpio_write");
    }
}
