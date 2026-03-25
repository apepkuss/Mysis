// 编译期互斥检查：确保只启用一个芯片 feature
#[cfg(all(feature = "esp32s3", feature = "esp32c3"))]
compile_error!("Cannot enable both `esp32s3` and `esp32c3` features");
#[cfg(all(feature = "esp32s3", feature = "esp32c6"))]
compile_error!("Cannot enable both `esp32s3` and `esp32c6` features");
#[cfg(all(feature = "esp32c3", feature = "esp32c6"))]
compile_error!("Cannot enable both `esp32c3` and `esp32c6` features");
#[cfg(not(any(feature = "esp32s3", feature = "esp32c3", feature = "esp32c6")))]
compile_error!("One of `esp32s3`, `esp32c3`, or `esp32c6` feature must be enabled");

// --- 芯片型号 ---

#[cfg(feature = "esp32s3")]
pub const CHIP_MODEL: &str = "esp32s3";
#[cfg(feature = "esp32c3")]
pub const CHIP_MODEL: &str = "esp32c3";
#[cfg(feature = "esp32c6")]
pub const CHIP_MODEL: &str = "esp32c6";

// --- PSRAM ---

#[cfg(feature = "esp32s3")]
pub const HAS_PSRAM: bool = true;
#[cfg(any(feature = "esp32c3", feature = "esp32c6"))]
pub const HAS_PSRAM: bool = false;

// --- 对话历史轮数 ---

#[cfg(feature = "esp32s3")]
pub const MAX_HISTORY_ROUNDS: usize = 10;
#[cfg(feature = "esp32c3")]
pub const MAX_HISTORY_ROUNDS: usize = 5;
#[cfg(feature = "esp32c6")]
pub const MAX_HISTORY_ROUNDS: usize = 8;

// --- 单条消息上限（字节） ---

#[cfg(feature = "esp32s3")]
pub const MAX_MESSAGE_LEN: usize = 1024;
#[cfg(feature = "esp32c3")]
pub const MAX_MESSAGE_LEN: usize = 512;
#[cfg(feature = "esp32c6")]
pub const MAX_MESSAGE_LEN: usize = 768;

// --- MQTT 缓冲区 ---

#[cfg(feature = "esp32s3")]
pub const MQTT_RX_BUF_SIZE: usize = 8192;
#[cfg(feature = "esp32c3")]
pub const MQTT_RX_BUF_SIZE: usize = 2048;
#[cfg(feature = "esp32c6")]
pub const MQTT_RX_BUF_SIZE: usize = 4096;

#[cfg(feature = "esp32s3")]
pub const MQTT_TX_BUF_SIZE: usize = 8192;
#[cfg(feature = "esp32c3")]
pub const MQTT_TX_BUF_SIZE: usize = 2048;
#[cfg(feature = "esp32c6")]
pub const MQTT_TX_BUF_SIZE: usize = 4096;

// --- 默认 GPIO 引脚号 ---

#[cfg(feature = "esp32s3")]
pub const DEFAULT_GPIO_PIN: u8 = 13;
#[cfg(feature = "esp32c3")]
pub const DEFAULT_GPIO_PIN: u8 = 4;
#[cfg(feature = "esp32c6")]
pub const DEFAULT_GPIO_PIN: u8 = 4;

// --- 安全 GPIO 范围 ---

#[cfg(feature = "esp32s3")]
pub const SAFE_GPIO_RANGE: core::ops::RangeInclusive<u8> = 1..=48;
#[cfg(feature = "esp32c3")]
pub const SAFE_GPIO_RANGE: core::ops::RangeInclusive<u8> = 2..=10;
#[cfg(feature = "esp32c6")]
pub const SAFE_GPIO_RANGE: core::ops::RangeInclusive<u8> = 2..=10;
