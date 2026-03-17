#![allow(dead_code)]

use crate::memory_store::SqliteMemoryStore;
use mysis_core::protocol::*;

/// 处理 memory/store 请求
pub fn handle_memory_store(
    store: &mut SqliteMemoryStore,
    device_id: &str,
    req: &MemoryStoreRequest,
) -> Result<(), String> {
    store.store_memory(device_id, &req.category, &req.content, req.metadata.clone())
}

/// 处理 memory/recall 请求，返回 MemoryRecallResult
pub fn handle_memory_recall(
    store: &SqliteMemoryStore,
    device_id: &str,
    req: &MemoryRecallRequest,
) -> Result<MemoryRecallResult, String> {
    let memories = store.recall_memories(device_id, &req.query, req.limit)?;
    Ok(MemoryRecallResult {
        id: req.id.clone(),
        memories,
    })
}

/// 处理冷启动同步请求，返回 MemorySyncResponse
pub fn handle_memory_sync(
    store: &SqliteMemoryStore,
    device_id: &str,
) -> Result<MemorySyncResponse, String> {
    let preferences = store.get_preferences(device_id)?;
    let summary = store.generate_summary(device_id)?;
    Ok(MemorySyncResponse {
        id: format!("sync-{device_id}"),
        preferences,
        summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_store_request() {
        let mut store = SqliteMemoryStore::open(":memory:").unwrap();
        let req = MemoryStoreRequest {
            id: "mem-001".into(),
            category: "preference".into(),
            content: "默认灯=客厅".into(),
            metadata: Some(serde_json::json!({"key": "default_light", "value": "living_room"})),
        };
        let result = handle_memory_store(&mut store, "dev-01", &req);
        assert!(result.is_ok());
    }

    #[test]
    fn handle_recall_request() {
        let mut store = SqliteMemoryStore::open(":memory:").unwrap();
        store
            .store_memory("dev-01", "event", "2026-03-15 浇花5分钟", None)
            .unwrap();
        let req = MemoryRecallRequest {
            id: "mem-002".into(),
            query: "浇花".into(),
            limit: 3,
        };
        let result = handle_memory_recall(&store, "dev-01", &req).unwrap();
        assert!(!result.memories.is_empty());
    }

    #[test]
    fn handle_sync_request() {
        let mut store = SqliteMemoryStore::open(":memory:").unwrap();
        store
            .store_memory(
                "dev-01",
                "preference",
                "默认灯",
                Some(serde_json::json!({"key": "default_light", "value": "living_room"})),
            )
            .unwrap();
        let result = handle_memory_sync(&store, "dev-01").unwrap();
        assert!(!result.preferences.is_empty());
    }
}
