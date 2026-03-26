use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct BridgeConfig {
    pub mqtt: MqttConfig,
    pub llm: LlmConfig,
    #[serde(default)]
    pub devices: DevicesConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
}

#[derive(Debug, Deserialize)]
pub struct MemoryConfig {
    #[serde(default = "default_memory_db_path")]
    pub db_path: String,
    /// 嵌入模型目录（含 model.onnx 和 tokenizer.json）
    #[serde(default)]
    pub embedding_model_dir: Option<String>,
    /// 嵌入向量维度
    #[serde(default = "default_embedding_dim")]
    pub embedding_dim: usize,
    /// 向量召回相似度阈值
    #[serde(default = "default_similarity_threshold")]
    pub similarity_threshold: f32,
    /// 向量召回 Top-K
    #[serde(default = "default_recall_top_k")]
    pub recall_top_k: usize,
}

fn default_memory_db_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    format!("{home}/.mysis/memory.db")
}
fn default_embedding_dim() -> usize {
    384
}
fn default_similarity_threshold() -> f32 {
    0.5
}
fn default_recall_top_k() -> usize {
    5
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            db_path: default_memory_db_path(),
            embedding_model_dir: None,
            embedding_dim: default_embedding_dim(),
            similarity_threshold: default_similarity_threshold(),
            recall_top_k: default_recall_top_k(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct MqttConfig {
    pub broker: String,
    #[serde(default = "default_mqtt_port")]
    pub port: u16,
    #[serde(default = "default_client_id")]
    pub client_id: String,
}

fn default_mqtt_port() -> u16 {
    1883
}
fn default_client_id() -> String {
    "mysis-bridge".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfig {
    /// LLM 提供商：`openai`（默认）或 `claude`
    #[serde(default = "default_provider")]
    pub provider: String,
    pub base_url: String,
    #[serde(default = "default_model")]
    pub model: String,
    /// API 密钥（Claude 使用 x-api-key，OpenAI 使用 Bearer token）
    #[serde(default)]
    pub api_key: Option<String>,
    #[allow(dead_code)]
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_provider() -> String {
    "openai".into()
}
fn default_model() -> String {
    "default".into()
}
fn default_max_tokens() -> u32 {
    512
}
fn default_timeout() -> u64 {
    30
}

#[derive(Debug, Deserialize)]
pub struct DevicesConfig {
    #[serde(default = "default_heartbeat_timeout")]
    pub heartbeat_timeout_secs: u64,
}

fn default_heartbeat_timeout() -> u64 {
    120
}

impl Default for DevicesConfig {
    fn default() -> Self {
        Self {
            heartbeat_timeout_secs: default_heartbeat_timeout(),
        }
    }
}

impl BridgeConfig {
    pub fn from_file(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_default_config() {
        let toml_str = r#"
[mqtt]
broker = "localhost"
port = 1883
client_id = "mysis-bridge"

[llm]
base_url = "http://localhost:8000/v1"
model = "qwen3-8b"
max_tokens = 512
timeout_secs = 30

[devices]
heartbeat_timeout_secs = 120
"#;
        let config: BridgeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.mqtt.broker, "localhost");
        assert_eq!(config.mqtt.port, 1883);
        assert_eq!(config.llm.base_url, "http://localhost:8000/v1");
        assert_eq!(config.llm.model, "qwen3-8b");
        assert_eq!(config.devices.heartbeat_timeout_secs, 120);
    }

    #[test]
    fn default_values_applied() {
        let toml_str = r#"
[mqtt]
broker = "192.168.1.100"

[llm]
base_url = "http://localhost:8080/v1"
"#;
        let config: BridgeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.mqtt.port, 1883);
        assert_eq!(config.mqtt.client_id, "mysis-bridge");
        assert_eq!(config.llm.model, "default");
        assert_eq!(config.llm.max_tokens, 512);
        assert_eq!(config.llm.timeout_secs, 30);
        assert_eq!(config.devices.heartbeat_timeout_secs, 120);
    }
}
