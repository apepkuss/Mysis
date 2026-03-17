use esp_idf_svc::nvs::*;
use mysis_core::memory::Memory;

pub struct NvsMemory {
    nvs: EspNvs<NvsDefault>,
}

impl NvsMemory {
    pub fn new(partition: EspNvsPartition<NvsDefault>) -> Result<Self, String> {
        let nvs = EspNvs::new(partition, "mysis_mem", true)
            .map_err(|e| format!("NVS init failed: {e}"))?;
        Ok(Self { nvs })
    }

    /// 启动时一次性加载所有已知偏好键到 Vec
    pub fn load_all_preferences(&self) -> Result<Vec<(String, String)>, String> {
        // NVS 不支持枚举所有 key，使用预定义的键列表尝试读取
        let known_keys = [
            "preference:default_light",
            "preference:wake_word",
            "device_state:living_room_light",
            "device_state:bedroom_light",
            "device_state:garden_pump",
        ];

        let mut result = Vec::new();
        let mut buf = [0u8; 128];
        for key in &known_keys {
            if let Ok(Some(val)) = self.nvs.get_str(key, &mut buf) {
                let short_key = key.splitn(2, ':').nth(1).unwrap_or(key);
                result.push((short_key.to_string(), val.to_string()));
            }
        }
        Ok(result)
    }
}

impl Memory for NvsMemory {
    fn store(&mut self, category: &str, key: &str, value: &str) -> Result<(), String> {
        let nvs_key = format!("{category}:{key}");
        self.nvs
            .set_str(&nvs_key, value)
            .map_err(|e| format!("NVS write failed: {e}"))?;
        Ok(())
    }

    fn recall(&self, key: &str) -> Result<Option<String>, String> {
        for prefix in &["preference", "device_state", "alias"] {
            let nvs_key = format!("{prefix}:{key}");
            let mut buf = [0u8; 128];
            if let Ok(Some(val)) = self.nvs.get_str(&nvs_key, &mut buf) {
                return Ok(Some(val.to_string()));
            }
        }
        Ok(None)
    }

    fn list(&self, _category: &str) -> Result<Vec<(String, String)>, String> {
        // NVS 不支持枚举，返回空（依赖启动时的预加载）
        Ok(vec![])
    }

    fn forget(&mut self, key: &str) -> Result<bool, String> {
        for prefix in &["preference", "device_state", "alias"] {
            let nvs_key = format!("{prefix}:{key}");
            if self.nvs.remove(&nvs_key).is_ok() {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
