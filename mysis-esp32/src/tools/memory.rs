use mysis_core::error::ToolError;
use mysis_core::memory::Memory;
use mysis_core::tool::Tool;
use std::sync::{Arc, Mutex};

/// LLM 可调用的记忆存储工具
/// 当 LLM 发现用户偏好时，调用此工具写入 NVS
pub struct MemoryStoreTool<M: Memory> {
    memory: Arc<Mutex<M>>,
}

impl<M: Memory> MemoryStoreTool<M> {
    pub fn new(memory: Arc<Mutex<M>>) -> Self {
        Self { memory }
    }
}

impl<M: Memory> Tool for MemoryStoreTool<M> {
    fn name(&self) -> &str {
        "memory_store"
    }

    fn description(&self) -> &str {
        "存储用户偏好或设备状态到本地记忆。当你发现用户的习惯或偏好时使用此工具。"
    }

    fn parameters_schema(&self) -> &str {
        r#"{"type":"object","properties":{"category":{"type":"string","enum":["preference","device_state","alias"],"description":"记忆分类"},"key":{"type":"string","description":"记忆键名"},"value":{"type":"string","description":"记忆值"}},"required":["category","key","value"]}"#
    }

    fn execute(&mut self, params: &str) -> Result<String, ToolError> {
        let v: serde_json::Value =
            serde_json::from_str(params).map_err(|e| ToolError::InvalidParams(e.to_string()))?;

        let category = v["category"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidParams("missing 'category'".into()))?;
        let key = v["key"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidParams("missing 'key'".into()))?;
        let value = v["value"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidParams("missing 'value'".into()))?;

        let mut mem = self
            .memory
            .lock()
            .map_err(|e| ToolError::ExecutionFailed(format!("lock failed: {e}")))?;
        mem.store(category, key, value)
            .map_err(|e| ToolError::ExecutionFailed(e))?;

        Ok(format!(
            r#"{{"success":true,"category":"{category}","key":"{key}","value":"{value}"}}"#
        ))
    }
}

/// LLM 可调用的记忆召回工具
/// 当 LLM 需要查询本地记忆时使用
pub struct MemoryRecallTool<M: Memory> {
    memory: Arc<Mutex<M>>,
}

impl<M: Memory> MemoryRecallTool<M> {
    pub fn new(memory: Arc<Mutex<M>>) -> Self {
        Self { memory }
    }
}

impl<M: Memory> Tool for MemoryRecallTool<M> {
    fn name(&self) -> &str {
        "memory_recall"
    }

    fn description(&self) -> &str {
        "从本地记忆中查询信息。当你需要回忆用户偏好或设备状态时使用。"
    }

    fn parameters_schema(&self) -> &str {
        r#"{"type":"object","properties":{"key":{"type":"string","description":"要查询的记忆键名"}},"required":["key"]}"#
    }

    fn execute(&mut self, params: &str) -> Result<String, ToolError> {
        let v: serde_json::Value =
            serde_json::from_str(params).map_err(|e| ToolError::InvalidParams(e.to_string()))?;

        let key = v["key"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidParams("missing 'key'".into()))?;

        let mem = self
            .memory
            .lock()
            .map_err(|e| ToolError::ExecutionFailed(format!("lock failed: {e}")))?;
        let value = mem
            .recall(key)
            .map_err(|e| ToolError::ExecutionFailed(e))?;

        match value {
            Some(v) => Ok(format!(r#"{{"found":true,"key":"{key}","value":"{v}"}}"#)),
            None => Ok(format!(r#"{{"found":false,"key":"{key}"}}"#)),
        }
    }
}
