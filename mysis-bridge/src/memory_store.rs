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
                    scope TEXT NOT NULL DEFAULT 'device',
                    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                    last_accessed TIMESTAMP
                );

                CREATE TABLE IF NOT EXISTS embeddings (
                    memory_id INTEGER PRIMARY KEY REFERENCES memories(id) ON DELETE CASCADE,
                    vector BLOB NOT NULL
                );

                CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                    content,
                    tokenize='trigram'
                );",
            )
            .map_err(|e| format!("failed to create tables: {e}"))?;
        Ok(())
    }

    /// 根据 category 自动决定 scope
    fn scope_for_category(category: &str) -> &'static str {
        match category {
            "preference" | "alias" => "global",
            _ => "device",
        }
    }

    /// 存储一条语义记忆（自动设置 scope）
    pub fn store_memory(
        &mut self,
        device_id: &str,
        category: &str,
        content: &str,
        metadata: Option<serde_json::Value>,
    ) -> Result<i64, String> {
        let metadata_str = metadata.map(|m| m.to_string());
        let scope = Self::scope_for_category(category);
        self.conn
            .execute(
                "INSERT INTO memories (device_id, category, content, metadata, scope) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![device_id, category, content, metadata_str, scope],
            )
            .map_err(|e| format!("failed to store memory: {e}"))?;
        let memory_id = self.conn.last_insert_rowid();

        // 同步写入 FTS5 索引（content-sync 模式，rowid = memory_id）
        self.conn
            .execute(
                "INSERT INTO memories_fts (rowid, content) VALUES (?1, ?2)",
                params![memory_id, content],
            )
            .map_err(|e| format!("failed to index memory in FTS5: {e}"))?;

        Ok(memory_id)
    }

    /// 存储嵌入向量（与 memory_id 关联）
    pub fn store_embedding(&mut self, memory_id: i64, vector: &[f32]) -> Result<(), String> {
        let blob = vector_to_blob(vector);
        self.conn
            .execute(
                "INSERT OR REPLACE INTO embeddings (memory_id, vector) VALUES (?1, ?2)",
                params![memory_id, blob],
            )
            .map_err(|e| format!("failed to store embedding: {e}"))?;
        Ok(())
    }

    /// 向量召回：计算余弦相似度，返回 Top-K
    pub fn vector_recall(
        &self,
        device_id: &str,
        query_vector: &[f32],
        top_k: usize,
        threshold: f32,
    ) -> Result<Vec<MemoryEntry>, String> {
        // 加载该设备可见的所有嵌入（本设备 + global scope）
        let mut stmt = self
            .conn
            .prepare(
                "SELECT m.id, m.category, m.content, e.vector
                 FROM memories m
                 JOIN embeddings e ON m.id = e.memory_id
                 WHERE m.device_id = ?1 OR m.scope = 'global'",
            )
            .map_err(|e| format!("failed to prepare vector recall: {e}"))?;

        let mut candidates: Vec<(f32, MemoryEntry)> = stmt
            .query_map(params![device_id], |row| {
                let category: String = row.get(1)?;
                let content: String = row.get(2)?;
                let blob: Vec<u8> = row.get(3)?;
                Ok((category, content, blob))
            })
            .map_err(|e| format!("failed to query embeddings: {e}"))?
            .filter_map(|r| r.ok())
            .filter_map(|(category, content, blob)| {
                let stored_vec = blob_to_vector(&blob);
                let sim = crate::embedder::cosine_similarity(query_vector, &stored_vec);
                if sim >= threshold {
                    Some((
                        sim,
                        MemoryEntry {
                            category,
                            content,
                            relevance: sim,
                        },
                    ))
                } else {
                    None
                }
            })
            .collect();

        // 按相似度降序排序，取 Top-K
        candidates.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        candidates.truncate(top_k);

        Ok(candidates.into_iter().map(|(_, entry)| entry).collect())
    }

    /// FTS5 全文搜索召回记忆（BM25 排序，含 global scope）
    pub fn recall_memories(
        &self,
        device_id: &str,
        query: &str,
        limit: u32,
    ) -> Result<Vec<MemoryEntry>, String> {
        let keywords: Vec<&str> = query.split_whitespace().collect();
        if keywords.is_empty() {
            return Ok(vec![]);
        }

        // trigram tokenizer 需要至少 3 个字符，短查询回退到 LIKE
        let min_len = keywords
            .iter()
            .map(|k| k.chars().count())
            .min()
            .unwrap_or(0);
        if min_len < 3 {
            return self.recall_memories_like(device_id, &keywords, limit);
        }

        // 构建 FTS5 MATCH 表达式：每个关键词用 OR 连接（宽松匹配）
        let fts_query = keywords.join(" OR ");

        let mut stmt = self
            .conn
            .prepare(
                "SELECT m.category, m.content, bm25(memories_fts) AS rank, m.id
                 FROM memories_fts f
                 JOIN memories m ON m.id = f.rowid
                 WHERE memories_fts MATCH ?1
                   AND (m.device_id = ?2 OR m.scope = 'global')
                 ORDER BY rank
                 LIMIT ?3",
            )
            .map_err(|e| format!("failed to prepare FTS5 query: {e}"))?;

        let mut matched_ids: Vec<i64> = Vec::new();
        let entries: Vec<MemoryEntry> = stmt
            .query_map(params![fts_query, device_id, limit], |row| {
                let rank: f64 = row.get(2)?;
                let id: i64 = row.get(3)?;
                Ok((
                    id,
                    MemoryEntry {
                        category: row.get(0)?,
                        content: row.get(1)?,
                        relevance: (1.0 / (1.0 - rank)) as f32,
                    },
                ))
            })
            .map_err(|e| format!("failed to execute FTS5 query: {e}"))?
            .filter_map(|r| r.ok())
            .map(|(id, entry)| {
                matched_ids.push(id);
                entry
            })
            .collect();

        if !matched_ids.is_empty() {
            let ids: Vec<String> = matched_ids.iter().map(|id| id.to_string()).collect();
            let ids_str = ids.join(",");
            let _ = self.conn.execute(
                &format!(
                    "UPDATE memories SET last_accessed = CURRENT_TIMESTAMP WHERE id IN ({ids_str})"
                ),
                [],
            );
        }

        Ok(entries)
    }

    /// LIKE 回退：短查询时使用（trigram 需 >= 3 字符）
    fn recall_memories_like(
        &self,
        device_id: &str,
        keywords: &[&str],
        limit: u32,
    ) -> Result<Vec<MemoryEntry>, String> {
        let conditions: Vec<String> = keywords
            .iter()
            .enumerate()
            .map(|(i, _)| format!("m.content LIKE ?{}", i + 3))
            .collect();
        let where_clause = conditions.join(" AND ");

        let sql = format!(
            "SELECT m.category, m.content FROM memories m
             WHERE (m.device_id = ?1 OR m.scope = 'global') AND ({where_clause})
             ORDER BY m.created_at DESC
             LIMIT ?2"
        );

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| format!("failed to prepare LIKE query: {e}"))?;

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
            .map_err(|e| format!("failed to execute LIKE query: {e}"))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(entries)
    }

    /// 混合召回：向量搜索 + 关键词匹配，合并去重
    pub fn hybrid_recall(
        &self,
        device_id: &str,
        query: &str,
        query_vector: Option<&[f32]>,
        top_k: usize,
        threshold: f32,
    ) -> Result<Vec<MemoryEntry>, String> {
        let mut results: Vec<MemoryEntry> = Vec::new();
        let mut seen_contents: std::collections::HashSet<String> = std::collections::HashSet::new();

        // 路径 A：向量相似度搜索（如果有向量）
        if let Some(qv) = query_vector {
            let vector_results = self.vector_recall(device_id, qv, top_k, threshold)?;
            for entry in vector_results {
                seen_contents.insert(entry.content.clone());
                results.push(entry);
            }
        }

        // 路径 B：关键词匹配补充
        let keyword_results = self.recall_memories(device_id, query, top_k as u32)?;
        for entry in keyword_results {
            if seen_contents.insert(entry.content.clone()) {
                results.push(entry);
            }
        }

        // 按 relevance 降序排序，向量结果（relevance < 1.0 的实际相似度）优先
        results.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(top_k);

        Ok(results)
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

/// f32 向量 → BLOB（little-endian bytes）
fn vector_to_blob(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// BLOB → f32 向量
fn blob_to_vector(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
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
    fn multi_device_isolation_device_scope() {
        let mut store = test_store();
        // device_state 是 device scope，应隔离
        store
            .store_memory("dev-01", "device_state", "状态A", None)
            .unwrap();
        store
            .store_memory("dev-02", "device_state", "状态B", None)
            .unwrap();
        let results_01 = store.recall_memories("dev-01", "状态", 5).unwrap();
        let results_02 = store.recall_memories("dev-02", "状态", 5).unwrap();
        assert_eq!(results_01.len(), 1);
        assert!(results_01[0].content.contains("A"));
        assert_eq!(results_02.len(), 1);
        assert!(results_02[0].content.contains("B"));
    }

    #[test]
    fn global_scope_shared_across_devices() {
        let mut store = test_store();
        // preference 是 global scope，两个设备都能看到
        store
            .store_memory("dev-01", "preference", "偏好A", None)
            .unwrap();
        store
            .store_memory("dev-02", "preference", "偏好B", None)
            .unwrap();
        let results_01 = store.recall_memories("dev-01", "偏好", 5).unwrap();
        let results_02 = store.recall_memories("dev-02", "偏好", 5).unwrap();
        // 两个设备都能看到两条全局偏好
        assert_eq!(results_01.len(), 2);
        assert_eq!(results_02.len(), 2);
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
