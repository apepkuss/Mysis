/// 记忆存储抽象 — 隔离 NVS / SQLite 实现细节。
pub trait Memory {
    /// 存储一条记忆
    fn store(&mut self, category: &str, key: &str, value: &str) -> Result<(), String>;

    /// 按 key 召回一条记忆
    fn recall(&self, key: &str) -> Result<Option<String>, String>;

    /// 列出指定分类下的所有记忆
    fn list(&self, category: &str) -> Result<Vec<(String, String)>, String>;

    /// 遗忘一条记忆，返回是否成功删除
    fn forget(&mut self, key: &str) -> Result<bool, String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockMemory {
        entries: Vec<(String, String, String)>, // (category, key, value)
    }

    impl MockMemory {
        fn new() -> Self {
            Self { entries: vec![] }
        }
    }

    impl Memory for MockMemory {
        fn store(&mut self, category: &str, key: &str, value: &str) -> Result<(), String> {
            self.entries
                .push((category.into(), key.into(), value.into()));
            Ok(())
        }

        fn recall(&self, key: &str) -> Result<Option<String>, String> {
            Ok(self
                .entries
                .iter()
                .find(|(_, k, _)| k == key)
                .map(|(_, _, v)| v.clone()))
        }

        fn list(&self, category: &str) -> Result<Vec<(String, String)>, String> {
            Ok(self
                .entries
                .iter()
                .filter(|(c, _, _)| c == category)
                .map(|(_, k, v)| (k.clone(), v.clone()))
                .collect())
        }

        fn forget(&mut self, key: &str) -> Result<bool, String> {
            let before = self.entries.len();
            self.entries.retain(|(_, k, _)| k != key);
            Ok(self.entries.len() < before)
        }
    }

    #[test]
    fn memory_store_and_recall() {
        let mut mem = MockMemory::new();
        mem.store("preference", "default_light", "living_room_light")
            .unwrap();
        let val = mem.recall("default_light").unwrap();
        assert_eq!(val, Some("living_room_light".into()));
    }

    #[test]
    fn memory_recall_not_found() {
        let mem = MockMemory::new();
        let val = mem.recall("nonexistent").unwrap();
        assert_eq!(val, None);
    }

    #[test]
    fn memory_list_by_category() {
        let mut mem = MockMemory::new();
        mem.store("preference", "default_light", "living_room")
            .unwrap();
        mem.store("preference", "wake_word", "小虾").unwrap();
        mem.store("device_state", "light", "on").unwrap();
        let prefs = mem.list("preference").unwrap();
        assert_eq!(prefs.len(), 2);
    }

    #[test]
    fn memory_forget() {
        let mut mem = MockMemory::new();
        mem.store("preference", "default_light", "living_room")
            .unwrap();
        let removed = mem.forget("default_light").unwrap();
        assert!(removed);
        let val = mem.recall("default_light").unwrap();
        assert_eq!(val, None);
    }
}
