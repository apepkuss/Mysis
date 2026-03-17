#![allow(dead_code)]

use mysis_core::protocol::{MemoryEntry, MemoryPreference};
use rusqlite::{params, Connection};

pub struct SqliteMemoryStore {
    conn: Connection,
}

impl SqliteMemoryStore {
    /// 打开数据库（路径或 ":memory:"），自动建表
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("failed to open database: {e}"))?;
        let store = Self { conn };
        store.init_tables()?;
        Ok(store)
    }

    fn init_tables(&self) -> Result<(), String> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS conversations (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    device_id TEXT NOT NULL,
                    role TEXT NOT NULL,
                    content TEXT NOT NULL,
                    tool_calls TEXT,
                    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                );

                CREATE TABLE IF NOT EXISTS memories (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    device_id TEXT NOT NULL,
                    category TEXT NOT NULL,
                    content TEXT NOT NULL,
                    metadata TEXT,
                    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                    last_accessed TIMESTAMP
                );",
            )
            .map_err(|e| format!("failed to create tables: {e}"))?;
        Ok(())
    }

    /// 存储一条语义记忆
    pub fn store_memory(
        &mut self,
        device_id: &str,
        category: &str,
        content: &str,
        metadata: Option<serde_json::Value>,
    ) -> Result<(), String> {
        let metadata_str = metadata.map(|m| m.to_string());
        self.conn
            .execute(
                "INSERT INTO memories (device_id, category, content, metadata) VALUES (?1, ?2, ?3, ?4)",
                params![device_id, category, content, metadata_str],
            )
            .map_err(|e| format!("failed to store memory: {e}"))?;
        Ok(())
    }

    /// 关键字搜索召回记忆（LIKE 匹配，适合中文和小数据量场景）
    pub fn recall_memories(
        &self,
        device_id: &str,
        query: &str,
        limit: u32,
    ) -> Result<Vec<MemoryEntry>, String> {
        // 按空格拆分关键词，所有关键词都需匹配
        let keywords: Vec<&str> = query.split_whitespace().collect();
        if keywords.is_empty() {
            return Ok(vec![]);
        }

        // 构建 WHERE 子句：每个关键词都用 LIKE 匹配
        let conditions: Vec<String> = keywords
            .iter()
            .enumerate()
            .map(|(i, _)| format!("content LIKE ?{}", i + 3))
            .collect();
        let where_clause = conditions.join(" AND ");

        let sql = format!(
            "SELECT category, content FROM memories
             WHERE device_id = ?1 AND ({where_clause})
             ORDER BY created_at DESC
             LIMIT ?2"
        );

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| format!("failed to prepare recall query: {e}"))?;

        // 绑定参数
        let like_params: Vec<String> = keywords.iter().map(|k| format!("%{k}%")).collect();
        let mut param_values: Vec<&dyn rusqlite::types::ToSql> = Vec::new();
        param_values.push(&device_id);
        param_values.push(&limit);
        for p in &like_params {
            param_values.push(p);
        }

        let entries = stmt
            .query_map(rusqlite::params_from_iter(param_values), |row| {
                Ok(MemoryEntry {
                    category: row.get(0)?,
                    content: row.get(1)?,
                    relevance: 1.0,
                })
            })
            .map_err(|e| format!("failed to execute recall query: {e}"))?
            .filter_map(|r| r.ok())
            .collect();

        // 更新 last_accessed
        for kw in &like_params {
            let _ = self.conn.execute(
                "UPDATE memories SET last_accessed = CURRENT_TIMESTAMP
                 WHERE device_id = ?1 AND content LIKE ?2",
                params![device_id, kw],
            );
        }

        Ok(entries)
    }

    /// 存储一条对话记录
    pub fn store_conversation(
        &mut self,
        device_id: &str,
        role: &str,
        content: &str,
        tool_calls: Option<String>,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO conversations (device_id, role, content, tool_calls) VALUES (?1, ?2, ?3, ?4)",
                params![device_id, role, content, tool_calls],
            )
            .map_err(|e| format!("failed to store conversation: {e}"))?;
        Ok(())
    }

    /// 获取最近 N 条对话
    pub fn recent_conversations(
        &self,
        device_id: &str,
        limit: u32,
    ) -> Result<Vec<(String, String)>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT role, content FROM conversations
                 WHERE device_id = ?1
                 ORDER BY created_at DESC
                 LIMIT ?2",
            )
            .map_err(|e| format!("failed to prepare conversations query: {e}"))?;

        let conversations = stmt
            .query_map(params![device_id, limit], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("failed to query conversations: {e}"))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(conversations)
    }

    /// 获取设备的所有偏好（用于冷启动同步）
    pub fn get_preferences(&self, device_id: &str) -> Result<Vec<MemoryPreference>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT metadata FROM memories
                 WHERE device_id = ?1 AND category = 'preference' AND metadata IS NOT NULL
                 ORDER BY created_at DESC",
            )
            .map_err(|e| format!("failed to prepare preferences query: {e}"))?;

        let prefs = stmt
            .query_map(params![device_id], |row| {
                let metadata_str: String = row.get(0)?;
                Ok(metadata_str)
            })
            .map_err(|e| format!("failed to query preferences: {e}"))?
            .filter_map(|r| r.ok())
            .filter_map(|metadata_str| {
                let v: serde_json::Value = serde_json::from_str(&metadata_str).ok()?;
                Some(MemoryPreference {
                    key: v["key"].as_str()?.to_string(),
                    value: v["value"].as_str()?.to_string(),
                })
            })
            .collect();

        Ok(prefs)
    }

    /// 生成近期对话摘要文本（用于冷启动同步）
    pub fn generate_summary(&self, device_id: &str) -> Result<String, String> {
        let conversations = self.recent_conversations(device_id, 5)?;
        if conversations.is_empty() {
            return Ok("无近期对话记录".into());
        }

        let summary: Vec<String> = conversations
            .iter()
            .rev()
            .map(|(role, content)| {
                let truncated = if content.len() > 50 {
                    format!("{}...", &content[..content.floor_char_boundary(50)])
                } else {
                    content.clone()
                };
                format!("{role}: {truncated}")
            })
            .collect();

        Ok(summary.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> SqliteMemoryStore {
        SqliteMemoryStore::open(":memory:").unwrap()
    }

    #[test]
    fn create_and_recall() {
        let mut store = test_store();
        store
            .store_memory("dev-01", "preference", "用户说灯指客厅灯", None)
            .unwrap();
        let results = store.recall_memories("dev-01", "客厅灯", 5).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("客厅灯"));
    }

    #[test]
    fn recall_empty() {
        let store = test_store();
        let results = store.recall_memories("dev-01", "不存在的内容", 5).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn store_conversation() {
        let mut store = test_store();
        store
            .store_conversation("dev-01", "user", "把灯打开", None)
            .unwrap();
        store
            .store_conversation("dev-01", "assistant", "好的", None)
            .unwrap();
        let conversations = store.recent_conversations("dev-01", 10).unwrap();
        assert_eq!(conversations.len(), 2);
    }

    #[test]
    fn get_preferences_for_sync() {
        let mut store = test_store();
        store
            .store_memory(
                "dev-01",
                "preference",
                "默认灯=客厅",
                Some(serde_json::json!({"key": "default_light", "value": "living_room"})),
            )
            .unwrap();
        let prefs = store.get_preferences("dev-01").unwrap();
        assert_eq!(prefs.len(), 1);
        assert_eq!(prefs[0].key, "default_light");
    }

    #[test]
    fn multi_device_isolation() {
        let mut store = test_store();
        store
            .store_memory("dev-01", "preference", "记忆A", None)
            .unwrap();
        store
            .store_memory("dev-02", "preference", "记忆B", None)
            .unwrap();
        let results_01 = store.recall_memories("dev-01", "记忆", 5).unwrap();
        let results_02 = store.recall_memories("dev-02", "记忆", 5).unwrap();
        assert_eq!(results_01.len(), 1);
        assert!(results_01[0].content.contains("A"));
        assert_eq!(results_02.len(), 1);
        assert!(results_02[0].content.contains("B"));
    }

    #[test]
    fn generate_summary_empty() {
        let store = test_store();
        let summary = store.generate_summary("dev-01").unwrap();
        assert_eq!(summary, "无近期对话记录");
    }

    #[test]
    fn generate_summary_with_conversations() {
        let mut store = test_store();
        store
            .store_conversation("dev-01", "user", "开灯", None)
            .unwrap();
        store
            .store_conversation("dev-01", "assistant", "客厅灯已打开", None)
            .unwrap();
        let summary = store.generate_summary("dev-01").unwrap();
        assert!(summary.contains("user"));
        assert!(summary.contains("assistant"));
    }
}
