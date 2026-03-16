use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct BridgeConfig {
    pub mqtt: MqttConfig,
    pub llm: LlmConfig,
    #[serde(default)]
    pub devices: DevicesConfig,
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

#[derive(Debug, Deserialize)]
pub struct LlmConfig {
    pub base_url: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[allow(dead_code)]
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
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
